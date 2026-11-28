use super::*;
use serde_json::json;
use std::path::Path;

fn args(value: Value) -> Args {
    serde_json::from_value(value).unwrap()
}

async fn run(bin: &Path, value: Value) -> Result<Value, String> {
    call_with(&args(value), bin).await.map(|text| serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// A stand-in for the nixa command: it notes every call and answers by the shape of the call.
fn fake_cli(dir: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join("nixa");
    std::fs::write(&bin, "#!/bin/sh\necho \"$@\" >> \"$(dirname \"$0\")/calls\"\ncase \" $* \" in\n  *\" status \"*) cat \"$(dirname \"$0\")/status.json\" ;;\n  *\" --apply \"*) cat \"$(dirname \"$0\")/apply.json\" ;;\n  *) cat \"$(dirname \"$0\")/dry.json\" ;;\nesac\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(dir.join("status.json"), r#"[{"agent":"claude","found":true,"file":"/x/.claude/settings.json","state":"not installed","have":0,"want":10,"others":[]},{"agent":"codex","found":true,"file":"/x/.codex/hooks.json","state":"installed","have":8,"want":8,"others":[]}]"#).unwrap();
    std::fs::write(dir.join("dry.json"), r#"[{"agent":"claude","found":true,"file":"/x/.claude/settings.json","changed":true,"written":false,"added":["Stop done","PreToolUse working"],"removed":[],"backup":null},{"agent":"codex","found":true,"file":"/x/.codex/hooks.json","changed":false,"written":false,"added":[],"removed":[],"backup":null},{"agent":"omp","found":false,"file":"/x/.omp/agent","dir":"/x/.omp/agent"}]"#).unwrap();
    bin
}

fn calls(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("calls")).unwrap_or_default().lines().map(str::to_string).collect()
}

fn fixture() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[tokio::test]
async fn status_reads_every_agent_with_json_and_never_applies() {
    let dir = fixture();
    let bin = fake_cli(dir.path());
    let out = run(&bin, json!({"action": "status"})).await.unwrap();
    assert_eq!(out["action"], "status");
    assert_eq!(out["apply"], false);
    assert_eq!(out["agents"].as_array().unwrap().len(), 2);
    assert_eq!(out["agents"][0]["state"], "not installed");
    assert_eq!(out["agents"][1]["state"], "installed");
    assert_eq!(calls(dir.path()), ["hooks status --json claude codex omp cursor"]);
    let one = run(&bin, json!({"action": "status", "agents": ["Codex"], "apply": true})).await.unwrap();
    assert_eq!(one["apply"], false, "status never applies");
    assert_eq!(calls(dir.path())[1], "hooks status --json codex", "agents are lower-cased and passed on");
}

#[tokio::test]
async fn install_and_remove_are_a_dry_run_until_apply() {
    let dir = fixture();
    let bin = fake_cli(dir.path());
    let dry = run(&bin, json!({"action": "install", "agents": ["claude", "omp"]})).await.unwrap();
    assert_eq!(dry["apply"], false);
    assert!(dry["note"].as_str().unwrap().starts_with("Dry run: nothing was written"));
    assert_eq!(dry["agents"][0]["changed"], true);
    assert_eq!(calls(dir.path()), ["hooks install --json claude omp"], "no --apply in a dry run");

    std::fs::write(dir.path().join("apply.json"), r#"[{"agent":"claude","found":true,"file":"/x/.claude/settings.json","changed":true,"written":true,"added":["Stop done"],"removed":[],"backup":"/x/.claude/settings.json.nixa-backup-1"},{"agent":"codex","found":true,"file":"/x/.codex/hooks.json","changed":true,"written":true,"added":[],"removed":["Stop done"],"backup":null},{"agent":"omp","found":false},{"agent":"cursor","agent":"cursor","error":"cannot merge"}]"#).unwrap();
    let applied = run(&bin, json!({"action": "install", "apply": true})).await.unwrap();
    assert_eq!(applied["status"], "partial", "one agent had an error");
    assert!(applied["note"].as_str().unwrap().contains("Not changed: cursor"));
    assert_eq!(applied["agents"][0]["state"], "not installed", "the status after the change is read back");
    assert_eq!(
        calls(dir.path())[1..],
        ["hooks install --json claude codex omp cursor", "hooks install --json claude codex omp cursor --apply", "hooks status --json claude codex omp cursor"]
    );

    std::fs::write(dir.path().join("apply.json"), r#"[{"agent":"cursor","found":true,"file":"/x/.cursor/hooks.json","changed":true,"written":true,"added":[],"removed":["Stop done"],"backup":"/x/.cursor/hooks.json.nixa-backup-1"}]"#).unwrap();
    let removed = run(&bin, json!({"action": "remove", "apply": true, "agents": ["cursor"]})).await.unwrap();
    assert_eq!(removed["status"], "applied");
    assert!(removed["note"].as_str().unwrap().starts_with("Applied to cursor."));
    assert_eq!(calls(dir.path()).last().unwrap(), "hooks status --json cursor");
}

#[tokio::test]
async fn bad_input_changes_nothing() {
    let dir = fixture();
    let bin = fake_cli(dir.path());
    assert!(run(&bin, json!({"action": "install", "agents": ["kimi"]})).await.unwrap_err().contains("kimi"));
    assert!(run(&bin, json!({"action": "fix"})).await.unwrap_err().contains("no action"));
    assert!(run(&bin, json!({})).await.unwrap_err().contains("action is needed"));
    assert!(!dir.path().join("calls").exists(), "nothing ran");
    assert!(run(&bin, json!({"action": "remove"})).await.is_ok());
}

#[test]
fn preview_lists_the_change_per_agent() {
    let report = serde_json::from_str::<Vec<Value>>(r#"[{"agent":"claude","found":true,"file":"/x/.claude/settings.json","changed":true,"added":["Stop done","PreToolUse working"],"removed":[]},{"agent":"codex","found":true,"file":"/x/.codex/hooks.json","changed":false},{"agent":"omp","found":false},{"agent":"cursor","error":"cannot merge"}]"#).unwrap();
    let text = preview("install", &report);
    assert!(text.starts_with("Install Nixa status hooks:\n- claude: /x/.claude/settings.json: + Stop done; + PreToolUse working"));
    assert!(text.contains("- codex: already as asked, no change (/x/.codex/hooks.json)"));
    assert!(text.contains("- omp: not on this Mac, skipped"));
    assert!(text.contains("- cursor: cannot, cannot merge"));
    assert!(preview("remove", &report).starts_with("Remove Nixa status hooks:"));
    let link = serde_json::from_str::<Vec<Value>>(r#"[{"agent":"omp","found":true,"file":"/x/.omp/agent/extensions/nixa-status.ts","changed":true,"changes":["+ link /x -> /y"]}]"#).unwrap();
    assert!(preview("install", &link).contains("- omp: /x/.omp/agent/extensions/nixa-status.ts: + link /x -> /y"));
}

#[test]
fn agents_are_picked_case_insensitively_and_deduplicated() {
    assert_eq!(pick_agents(&[]).unwrap(), AGENTS.iter().map(|a| a.to_string()).collect::<Vec<_>>());
    assert_eq!(pick_agents(&["Claude".into(), "claude".into(), "omp".into()]).unwrap(), ["claude", "omp"]);
    assert!(pick_agents(&["cursor".into(), "nope".into()]).is_err());
}

#[tokio::test]
async fn run_reports_a_silent_command_and_a_missing_one() {
    let dir = tempfile::tempdir().unwrap();
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.path().join("nixa");
    std::fs::write(&bin, "#!/bin/sh\necho 'not json' >&2\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let error = super::run(&bin, "status", &["claude".into()], false).await.unwrap_err();
    assert!(error.contains("gave no report: not json"), "{error}");
    let missing = dir.path().join("gone");
    assert!(super::run(&missing, "status", &[], false).await.unwrap_err().contains("could not run"));
}
