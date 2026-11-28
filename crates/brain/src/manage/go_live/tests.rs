use super::outcome::{self, NOT_LIVE, Outcome, Stage, State};
use super::runner;
use super::pick_repo;
use crate::app::{App, lock};
use crate::util::now_ms;
use std::path::Path;

fn never(_: u32) -> bool {
    false
}

#[tokio::test]
async fn release_wait_obeys_off_without_starting_a_process() {
    let root = tempfile::tempdir().unwrap();
    let app = App::open(root.path()).unwrap();
    let dir = root.path().join(outcome::FOLDER);
    std::fs::create_dir(&dir).unwrap();
    outcome::write(&dir, &going(Stage::Restart)).unwrap();
    crate::settings_api::set_nixa(&app, false).unwrap();
    assert!(super::wait_result(&app).await.unwrap_err().contains("Nixa is off"));
    assert!(dir.join("cancel").is_file());
}

fn always(_: u32) -> bool {
    true
}

fn going(stage: Stage) -> Outcome {
    Outcome { stage, started_at: 1_000, launched_at: 2_000, pid: 42, task: Some(7), lines: vec!["Building.".into()], ..Outcome::default() }
}

fn make_repo(dir: &Path) {
    std::fs::write(dir.join("Cargo.lock"), "").unwrap();
    for part in ["crates/xtask", "crates/app"] {
        std::fs::create_dir_all(dir.join(part)).unwrap();
        std::fs::write(dir.join(part).join("Cargo.toml"), "").unwrap();
    }
}

#[test]
fn a_build_stage_runs_while_its_brain_lives() {
    let o = going(Stage::Build);
    assert!(outcome::running(&o, None, 5_000, &always));
    assert!(!outcome::running(&o, None, 5_000, &never));
    assert!(!outcome::running(&o, None, 1_000 + outcome::STALE_MS + 1, &always));
    let done = Outcome { state: State::Done, ..o };
    assert!(!outcome::running(&done, None, 5_000, &always));
}

#[test]
fn a_restart_stage_follows_the_runner_pid() {
    let o = going(Stage::Restart);
    assert!(outcome::running(&o, None, 2_000 + outcome::START_MS, &never));
    assert!(!outcome::running(&o, None, 2_000 + outcome::START_MS + 1, &never));
    assert!(outcome::running(&o, Some(99), 5_000_000, &always));
    assert!(!outcome::running(&o, Some(99), 5_000, &never));
}

#[test]
fn the_end_file_decides_the_result() {
    let dir = Path::new("/nowhere");
    let o = going(Stage::Restart);
    let ok = outcome::next_state(&o, Some(true), vec!["Installed.".into()], Some(9), 50_000, &always, dir).unwrap();
    assert_eq!(ok.state, State::Done);
    assert!(ok.ok);
    assert_eq!(ok.ended_at, 50_000);
    assert_eq!(ok.lines, vec!["Building.".to_string(), "Installed.".to_string()]);
    let bad = outcome::next_state(&o, Some(false), vec![format!("{NOT_LIVE} copy failed.")], None, 50_000, &never, dir).unwrap();
    assert_eq!(bad.state, State::NotLive);
    assert!(outcome::text(&bad).contains("copy failed"), "{}", outcome::text(&bad));
    let quiet = outcome::next_state(&o, Some(false), Vec::new(), None, 50_000, &never, dir).unwrap();
    assert!(quiet.lines.last().unwrap().starts_with(NOT_LIVE));
    assert_eq!(outcome::next_state(&o, None, Vec::new(), Some(9), 50_000, &always, dir), None);
    let lost = outcome::next_state(&o, None, Vec::new(), Some(9), 50_000, &never, dir).unwrap();
    assert_eq!(lost.state, State::NotLive);
    assert!(outcome::text(&lost).contains("stopped without a result"));
}

#[test]
fn settle_reports_once_and_keeps_the_state() {
    let dir = tempfile::tempdir().unwrap();
    let o = going(Stage::Restart);
    outcome::write(dir.path(), &o).unwrap();
    std::fs::write(dir.path().join(outcome::STEPS_FILE), "Nixa quit.\nInstalled.\n\n").unwrap();
    assert_eq!(outcome::settle(dir.path(), 3_000, &always), None);
    std::fs::write(dir.path().join(outcome::END_FILE), "ok\n").unwrap();
    let done = outcome::settle(dir.path(), 3_000, &always).unwrap();
    assert!(done.reported && done.ok);
    assert_eq!(done.state, State::Done);
    assert_eq!(outcome::read(dir.path()).unwrap(), done);
    assert_eq!(outcome::settle(dir.path(), 4_000, &always), None);
    let text = std::fs::read_to_string(dir.path().join(outcome::OUTCOME_FILE)).unwrap();
    assert!(text.contains("\"state\": \"done\""), "{text}");
}

#[test]
fn texts_name_the_task_and_the_place() {
    let done = Outcome { state: State::Done, ok: true, task: Some(3), ..Outcome::default() };
    let text = outcome::text(&done);
    assert!(text.contains("task #3") && text.contains(runner::APP_DEST) && text.contains("No git step") && text.contains("retained for rollback"), "{text}");
    let running = Outcome::default();
    assert!(outcome::text(&running).contains("Nothing is live yet"));
    assert_eq!(outcome::task_words(None), "");
}

#[test]
fn the_runner_pid_must_be_a_real_number() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(outcome::runner_pid(dir.path()), None);
    std::fs::write(dir.path().join(outcome::PID_FILE), "123\n").unwrap();
    assert_eq!(outcome::runner_pid(dir.path()), Some(123));
    std::fs::write(dir.path().join(outcome::PID_FILE), "1").unwrap();
    assert_eq!(outcome::runner_pid(dir.path()), None);
}

#[test]
fn the_repo_must_be_the_nixa_source() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    assert!(pick_repo(None, None, None, None).unwrap_err().contains("Give repo"));
    assert!(pick_repo(Some(root.to_str().unwrap()), None, None, None).unwrap_err().contains("not the Nixa source folder"));
    make_repo(&root);
    assert_eq!(pick_repo(Some(root.to_str().unwrap()), None, None, None).unwrap(), root);
    assert_eq!(pick_repo(None, Some(root.clone()), None, None).unwrap(), root);
    assert_eq!(pick_repo(None, Some(root.join("gone")), Some(root.clone()), None).unwrap(), root);
    assert!(pick_repo(Some("relative/path"), None, None, None).is_err());
    let home = root.parent().unwrap();
    let name = root.file_name().unwrap().to_str().unwrap();
    assert_eq!(pick_repo(Some(&format!("~/{name}")), None, None, Some(home)).unwrap(), root);
}

#[test]
fn cargo_is_found_on_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    assert_eq!(runner::find_cargo(&format!("relative:{}", bin.display())), None);
    std::fs::write(bin.join("cargo"), "").unwrap();
    assert_eq!(runner::find_cargo(&format!("/nowhere:{}", bin.display())), Some(bin.join("cargo")));
}

#[test]
fn legacy_start_is_refused_before_touching_the_staging_folder() {
    let root = tempfile::tempdir().unwrap();
    let app = App::open(root.path()).unwrap();
    lock(&app.brain).last_user = Some(("yes, go live".into(), now_ms()));
    let args = serde_json::json!({"action": "start"});
    let reply = super::call(&app, args.as_object().unwrap()).unwrap();
    let result: serde_json::Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(result["status"], "refused");
    assert!(!root.path().join(outcome::FOLDER).exists());
}

#[test]
fn staging_never_uses_an_old_install_result_and_status_does_not_write() {
    let root = tempfile::tempdir().unwrap();
    let app = App::open(root.path()).unwrap();
    let dir = root.path().join(outcome::FOLDER);
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join(outcome::END_FILE), "ok\n").unwrap();
    let path = dir.join("candidate-1/Nixa.app");
    let record = Outcome { candidate: Some(path.clone()), pid: std::process::id(), started_at: now_ms(), ..going(Stage::Staging) };
    outcome::write(&dir, &record).unwrap();
    let result: serde_json::Value = serde_json::from_str(&super::status(&app)).unwrap();
    assert_eq!(result["activationAvailable"], true);
    assert_eq!(result["running"], true);
    assert_eq!(result["last"]["state"], "running");
    assert_eq!(result["last"]["candidate"], path.to_str().unwrap());
    assert_eq!(outcome::read(&dir), Some(record.clone()));
    let interrupted = outcome::observe(&dir, &record, now_ms(), &never).unwrap();
    assert_eq!(interrupted.state, State::NotLive);
    assert!(!interrupted.ok);
}

#[tokio::test]
async fn an_approved_stage_runs_cargo_and_keeps_a_private_candidate() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let repo = root.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    make_repo(&repo);
    std::fs::write(repo.join("Cargo.toml"), "[workspace]\nresolver = '2'\nmembers = ['crates/xtask']\n").unwrap();
    let xtask = repo.join("crates/xtask");
    std::fs::write(xtask.join("Cargo.toml"), "[package]\nname = 'xtask'\nversion = '0.0.0'\nedition = '2024'\n").unwrap();
    std::fs::create_dir(xtask.join("src")).unwrap();
    std::fs::write(xtask.join("src/main.rs"), r#"
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    assert_eq!(args.len(), 4);
    assert_eq!(args[0], "bundle");
    assert_eq!(args[1], "--no-live-check");
    assert_eq!(args[2], "--out");
    assert!(std::env::var_os("NIXA_DATA_DIR").is_none());
    let path = std::path::Path::new(&args[3]).join("Nixa.app/Contents/MacOS");
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join("nixa-app"), b"staged fixture").unwrap();
}
"#).unwrap();
    let app = App::open(&data).unwrap();
    let request = serde_json::json!({"action": "stage", "repo": repo});
    let args = request.as_object().unwrap();
    let dispatch = || serde_json::from_str::<serde_json::Value>(&super::call(&app, args).unwrap()).unwrap();
    let release = data.join(outcome::FOLDER);
    let result = dispatch();
    assert_eq!(result["status"], "staging", "{result}");
    let record = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let record = outcome::read(&release).unwrap();
            if record.state != State::Running {
                break record;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    assert_eq!(record.state, State::Staged, "{:?}", record.lines);
    assert!(!record.ok);
    let bundle = record.candidate.unwrap();
    assert!(bundle.starts_with(&release));
    assert_eq!(std::fs::read(bundle.join("Contents/MacOS/nixa-app")).unwrap(), b"staged fixture");
    assert!(bundle.parent().unwrap().join(outcome::BUILD_LOG).is_file());
}
