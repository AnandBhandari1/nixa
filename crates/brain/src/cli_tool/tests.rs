use super::*;

fn words(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_string).collect()
}

fn kind(line: &str) -> Kind {
    classify(&words(line))
}

fn refused(line: &str) -> bool {
    matches!(kind(line), Kind::Refuse(_))
}

#[test]
fn read_forms_run_at_once() {
    for line in [
        "version", "--version", "help", "--skill", "privacy", "term ls", "t read 3", "term wait 2 --secs 5", "task ls --space x",
        "task workflows list", "task trees walk a", "space ls", "deep digest", "voice devices", "phone status", "settings",
        "settings show", "settings --json", "models", "models ls", "models teams ls", "chat history", "chat memory ls",
        "chat history search build", "chat search release --source docs", "chat memory search tea",
        "app state", "tool list", "--json task ls", "rules", "rules show", "rules standing", "rules answers ls",
        "rules answers check run tests", "rules trees", "rules trees read watch-agents", "rules trees walk",
    ] {
        assert_eq!(kind(line), Kind::Read, "{line}");
    }
}

#[test]
fn other_forms_are_writes() {
    for line in [
        "term kill 3", "task start x", "task workflows run x", "settings privacy on", "settings web on",
        "models teams add a", "chat memory add x", "chat memory forget x", "task edit 3 --title x", "space add x", "deep start", "hooks install",
        "task --space ls kill", "rules standing set --file f --hash h", "rules answers rm 3",
        "rules trees delete x", "tool call x",
    ] {
        assert_eq!(kind(line), Kind::Write, "{line}");
    }
}

#[test]
fn hard_refusals_hold_even_with_flags() {
    for line in [
        "chat say hi", "chat --json say hi", "phone key", "engine stop", "engine restart", "brain stop",
        "hooks install --apply", "hooks remove claude --apply", "connector secret set x", "settings web key put",
        "--socket /tmp/x task ls", "task ls --socket=/tmp/x", "t ls --socket a",
    ] {
        assert!(refused(line), "{line}");
    }
    assert!(!refused("hooks install"));
}

#[test]
fn dangerous_commands_are_refused() {
    for line in [
        "file read ~/.codex/auth.json", "file read /etc/passwd", "file write a.txt", "file edit a.txt",
        "exec -- /usr/bin/security find-generic-password -w -s x", "exec ls", "gh auth token", "gh pr list",
        "heavy --tool /bin/sh", "heavy -- /bin/sh -c id", "hooks add claude --apply", "hooks rm claude --apply",
        "hooks uninstall --apply", "hooks install --apply", "connector ls", "unknown", "--json file read x",
        "--data-dir /tmp task ls",
    ] {
        assert!(refused(line), "{line}");
    }
}

#[test]
fn writes_wait_for_a_yes_that_names_them() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let args = words("task start x");
    assert!(ask(&app, &args).is_some());
    let later = crate::util::now_ms() + 10;
    crate::app::lock(&app.brain).last_user = Some(("yes".into(), later));
    assert!(ask(&app, &args).is_some());
    crate::app::lock(&app.brain).last_user = Some(("yes nixa".into(), later + 1));
    assert!(ask(&app, &words("task start y")).is_some());
    assert!(ask(&app, &args).is_some());
    crate::app::lock(&app.brain).last_user = Some(("yes task".into(), crate::util::now_ms() + 20));
    assert_eq!(ask(&app, &args), None);
    assert!(ask(&app, &args).is_some());
}

fn input(value: Value) -> Args {
    value.as_object().unwrap().clone()
}

#[test]
fn arguments_are_built_plainly() {
    let w = wanted(&input(json!({"args": ["nixa", "task", "ls"], "json": true, "timeout_secs": 9999}))).unwrap();
    assert_eq!(w.args, words("--json task ls"));
    assert_eq!(w.secs, MAX_SECS);
    let w = wanted(&input(json!({"args": ["task", "ls", "--json"], "json": true}))).unwrap();
    assert_eq!(w.args, words("task ls --json"));
    assert_eq!(w.secs, DEFAULT_SECS);
    let w = wanted(&input(json!({"args": ["exec", "cat"], "stdin": "hi"}))).unwrap();
    assert_eq!((w.args, w.stdin), (words("exec cat"), Some("hi".into())));
    assert!(wanted(&input(json!({"args": []}))).is_err());
    assert!(wanted(&input(json!({"args": ["task", 3]}))).is_err());
    assert!(wanted(&input(json!({"args": ["x"], "cmd": "y"}))).is_err());
}

#[test]
fn long_output_is_cut_and_said() {
    let (text, was_cut) = cut(&"é".repeat(OUT_CHARS + 5), OUT_CHARS);
    assert!(was_cut);
    assert_eq!(text.chars().count(), OUT_CHARS);
    assert_eq!(cut("short", 10), ("short".into(), false));
}

#[tokio::test]
async fn real_cli_runs_against_a_temp_folder() {
    let bin = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/nixa");
    if !bin.is_file() {
        eprintln!("skip: build the debug nixa first");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let target = Target { dir: dir.path().to_path_buf(), socket: dir.path().join(paths::SOCKET_FILE), brain: "127.0.0.1:9".into() };
    for line in ["version", "task ls --space x"] {
        let w = Wanted { args: words(line), stdin: None, secs: 20 };
        let ran = run(&bin, &target, &w).await.unwrap();
        println!("nixa {line} -> exit {:?} in {} ms\nstdout: {}\nstderr: {}", ran.code, ran.millis, ran.stdout.trim(), ran.stderr.trim());
        assert!(!ran.timed_out);
        if line == "version" {
            assert_eq!(ran.code, Some(0));
            assert!(ran.stdout.starts_with("nixa "));
        }
    }
}
