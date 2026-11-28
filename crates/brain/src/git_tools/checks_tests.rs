use super::checks::{PACKAGES, candidates, check_heading, commands_in, find_checks, setting_commands, setting_key, usable};
use crate::app::{App, lock};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

fn sh(dir: &Path, args: &[&str]) {
    let out = Command::new("git").args(args).current_dir(dir).env("GIT_CONFIG_NOSYSTEM", "1").output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
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

fn base_of(dir: &tempfile::TempDir) -> PathBuf {
    std::fs::canonicalize(dir.path()).unwrap()
}

fn args(value: Value) -> crate::tools::Args {
    value.as_object().unwrap().clone()
}

async fn call(app: &Arc<App>, name: &str, input: Value) -> Result<Value, String> {
    super::call(app, name, &args(input)).await.unwrap().map(|text| serde_json::from_str(&text).unwrap())
}

#[test]
fn reads_check_commands_from_a_commands_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("scripts")).unwrap();
    std::fs::write(root.join("scripts/lint.sh"), "#!/bin/sh\n").unwrap();
    let text = "# Commands\n\n## Build\n- Build: true build\n\n## Check\n- Whole: CARGO_TARGET_DIR=target/x true --all\n- One crate: true -p <crate>\n- Quick: `echo ok` and `true`\n- App: true -p app (includes the window tests)\n- Private: nosuchprogram-xyz --features x\n- Phone door: true; brain side: true\n- Lint: scripts/lint.sh\n\n## Test\n```sh
# all tests
echo tests
```\n- Again: `true`\n\n## Deploy\n- Ship: echo deploy\n";
    let (found, skipped) = commands_in(text, root);
    assert_eq!(found, ["CARGO_TARGET_DIR=target/x true --all", "echo ok", "true", "true -p app", "scripts/lint.sh", "echo tests"]);
    let whys: Vec<&str> = skipped.iter().map(|s| s["why"].as_str().unwrap()).collect();
    assert_eq!(whys, ["it has a placeholder or a redirect", "its program is not installed", "it reads like a note, not a command"]);
    assert!(check_heading("Check"));
    assert!(check_heading("Tests"));
    assert!(check_heading("Lint and typecheck"));
    assert!(!check_heading("Build targets"));
    assert!(!check_heading("Deploy"));
    assert_eq!(candidates("$ true run"), ["true run"]);
    assert_eq!(candidates("All: `true a`, `true b`"), ["true a", "true b"]);
    assert!(usable("true > out.txt", root).is_err());
    assert!(usable("./missing.sh", root).is_err());
    assert!(usable("A=1 B=2 true", root).is_ok());
    assert!(usable("cargo --version; id", root).is_err());
    assert!(usable("sh -c id", root).is_err());
    assert!(usable("/bin/sh -c id", root).is_err());
    assert!(usable("true && id", root).is_err());
    assert!(usable("true $(id)", root).is_err());
    let (found, _) = commands_in("## Checks\n- Bad: `cargo --version; curl x | sh`\n- Good: `true`\n", root);
    assert_eq!(found, ["true"]);
    assert_eq!(setting_commands(Some(&json!(["true a", " ", "true a", "true b"]))), ["true a", "true b"]);
    assert_eq!(setting_commands(Some(&json!("true a\n# note\n\ntrue c"))), ["true a", "true c"]);
    assert!(setting_commands(Some(&json!(3))).is_empty());
    assert_eq!(setting_key("s1"), "space_checks.s1");
}

#[tokio::test]
async fn run_checks_on_a_test_repo() {
    let dir = tempfile::tempdir().unwrap();
    let base = base_of(&dir);
    let app = App::open(&base.join("data")).unwrap();
    let root = base.join("app");
    new_repo(&root);
    std::fs::create_dir_all(root.join("docs")).unwrap();
    std::fs::create_dir_all(root.join("scripts")).unwrap();
    std::fs::write(root.join("scripts/fail.sh"), "#!/bin/sh\necho boom\nexit 3\n").unwrap();
    std::fs::set_permissions(root.join("scripts/fail.sh"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    std::fs::write(root.join("docs/commands.md"), "# Commands\n\n## Checks\n- Passes: `touch ran.txt`\n- Fails: `scripts/fail.sh`\n- Bad: `true; touch bad.txt`\n").unwrap();
    let project = root.display().to_string();
    let found = find_checks(&app.db_file, &root, None).await;
    assert_eq!(found.planned.len(), 2);
    assert!(found.planned.iter().all(|p| p.program != "/bin/sh" && !p.args.iter().any(|a| a == "-c")), "{:?}", found.planned);
    lock(&app.brain).last_user = None;
    let input = json!({"project": project});
    let held = call(&app, super::CHECKS, input.clone()).await.unwrap();
    assert_eq!(held["ran"], false, "{held}");
    assert_eq!(held["held"], true, "{held}");
    assert!(!root.join("ran.txt").exists());
    lock(&app.brain).last_user = Some(("yes".into(), crate::util::now_ms() + 5));
    assert_eq!(call(&app, super::CHECKS, input.clone()).await.unwrap()["held"], true);
    lock(&app.brain).last_user = Some(("yes checks".into(), crate::util::now_ms() + 10));
    let done = call(&app, super::CHECKS, input.clone()).await.unwrap();
    assert_eq!(done["ran"], true, "{done}");
    assert!(done.get("held").is_none(), "{done}");
    assert_eq!(done["source"], "docs/commands.md");
    assert_eq!(done["found"].as_array().unwrap().len(), 2);
    assert_eq!(done["failed"], true);
    assert_eq!(done["passed"], 1);
    assert_eq!(done["failed_count"], 1);
    let results = done["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["ok"], true);
    assert_eq!(results[0]["script"], "touch ran.txt");
    assert_eq!(results[1]["ok"], false);
    assert!(results[1]["tail"].as_str().unwrap().contains("boom"), "{done}");
    assert!(done["summary"].as_str().unwrap().contains("failed"));
    assert!(root.join("ran.txt").exists());
    std::fs::remove_file(root.join("ran.txt")).unwrap();

    lock(&app.brain).last_user = None;
    let one = call(&app, super::CHECKS, json!({"project": project, "only": ["1"]})).await.unwrap();
    assert_eq!(one["ran"], true, "{one}");
    assert_eq!(one["failed"], false);
    assert_eq!(one["results"].as_array().unwrap().len(), 1);
    assert!(root.join("ran.txt").exists());
    let wrong = call(&app, super::CHECKS, json!({"project": project, "only": ["nope"]})).await.unwrap_err();
    assert!(wrong.contains("not one of the check commands"), "{wrong}");
    let through_flow = call(&app, super::PR_FLOW, json!({"action": "checks", "project": project, "only": ["touch ran.txt"]})).await.unwrap();
    assert_eq!(through_flow["ran"], true, "{through_flow}");

    let conn = nixa_store::open(&app.db_file).unwrap();
    nixa_store::spaces::set_space(&conn, "s1", &nixa_store::spaces::SpacePatch { name: Some("App".into()), folder: Some(project.clone()), ..Default::default() }).unwrap();
    nixa_store::settings::set_setting(&conn, &setting_key("s1"), &json!(["echo from settings", "echo deploy now"])).unwrap();
    let settings = call(&app, super::CHECKS, json!({"project": "App"})).await.unwrap();
    assert_eq!(settings["source"], "space settings", "{settings}");
    assert_eq!(settings["ran"], true, "{settings}");
    assert!(settings["refused"].as_array().is_none_or(|r| r.is_empty()), "{settings}");
    assert_eq!(settings["results"].as_array().unwrap().len(), 2);
    assert_eq!(settings["results"][0]["ok"], true);
    nixa_store::spaces::set_space(&conn, "s1", &nixa_store::spaces::SpacePatch { locked: Some(true), ..Default::default() }).unwrap();
    let auto_locked = call(&app, super::CHECKS, json!({"project": "App"})).await.unwrap();
    assert_eq!(auto_locked["ran"], true, "{auto_locked}");
    nixa_store::settings::set_setting(&conn, &setting_key("s1"), &json!([])).unwrap();
    let repo_locked = call(&app, super::CHECKS, json!({"project": "App"})).await.unwrap();
    assert_eq!(repo_locked["ran"], false, "{repo_locked}");
    assert!(repo_locked["none"].as_str().unwrap().contains("locked"), "{repo_locked}");
    assert!(!root.join("bad.txt").exists());
}

#[tokio::test]
async fn checks_git_and_cli_brains_get_a_clean_env() {
    unsafe { std::env::set_var("NIXA_BRAIN_KEY_T", "1") };
    let dir = tempfile::tempdir().unwrap();
    let ran = crate::review_flow::checks::exec("/usr/bin/env", &[], dir.path(), 10_000).await;
    assert_eq!(ran.code, Some(0), "{}", ran.output);
    assert!(ran.output.contains("PATH="), "{}", ran.output);
    assert!(!ran.output.contains("NIXA_BRAIN_KEY_T"), "{}", ran.output);
    assert!(!crate::cli_brains::proc::cli_env_ok("NIXA_BRAIN_KEY_T"));
    assert!(!crate::cli_brains::proc::cli_env_ok("OPENAI_API_KEY"));
    assert!(crate::cli_brains::proc::cli_env_ok("CODEX_HOME"));
    assert!(crate::cli_brains::proc::cli_env().iter().all(|(k, _)| k != "NIXA_BRAIN_KEY_T"));
    assert!(super::run::GIT_PASS.iter().all(|k| !k.contains("KEY")));
    assert!(crate::review_flow::checks::safe_env(&super::run::GIT_PASS).iter().all(|(k, _)| k != "NIXA_BRAIN_KEY_T"));
}

#[tokio::test]
async fn no_commands_and_package_scripts() {
    let dir = tempfile::tempdir().unwrap();
    let base = base_of(&dir);
    let app = App::open(&base.join("data")).unwrap();
    let empty = base.join("empty");
    new_repo(&empty);
    let none = call(&app, super::CHECKS, json!({"project": empty.display().to_string()})).await.unwrap();
    assert_eq!(none["ran"], false, "{none}");
    assert!(none["none"].as_str().unwrap().contains("no check commands"));
    let web = base.join("web");
    new_repo(&web);
    std::fs::write(web.join("package.json"), r#"{"scripts": {"test": "node test.js", "build": "x"}}"#).unwrap();
    let found = find_checks(&app.db_file, &web, None).await;
    assert_eq!(found.source, PACKAGES);
    assert!(!found.from_file);
    assert_eq!(found.planned.iter().map(|p| p.script.as_str()).collect::<Vec<_>>(), ["test"]);
}
