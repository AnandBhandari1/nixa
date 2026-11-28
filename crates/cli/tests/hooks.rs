use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Home {
    dir: PathBuf,
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn home(folders: &[&str]) -> Home {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("nixa-hooks-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for f in folders {
        fs::create_dir_all(dir.join(f)).unwrap();
    }
    Home { dir }
}

fn nixa(h: &Home, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nixa"))
        .args(args)
        .arg("--home")
        .arg(&h.dir)
        .env("HOME", "/nonexistent-nixa-test-home")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .output()
        .unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).to_string()
}

fn backups(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().contains(".nixa-backup-"))
        .collect()
}

fn repo_hooks() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../hooks").canonicalize().unwrap()
}

const CLAUDE_START: &str = r#"{"zeta":"keep me","env":{"A":"1"},"hooks":{"Stop":[{"hooks":[{"type":"command","command":"say done"}]}]},"alpha":[1,2.50]}"#;

#[test]
fn claude_dry_run_then_apply_then_remove() {
    let h = home(&[".claude"]);
    let settings = h.dir.join(".claude/settings.json");
    fs::write(&settings, CLAUDE_START).unwrap();
    fs::set_permissions(&settings, fs::Permissions::from_mode(0o644)).unwrap();

    let dry = nixa(&h, &["hooks", "install", "claude"]);
    let out = text(&dry.stdout);
    assert_eq!(dry.status.code(), Some(0), "{out}{}", text(&dry.stderr));
    assert!(out.contains("+ Stop {\"hooks\":[{\"type\":\"command\",\"command\":"), "{out}");
    assert!(out.contains("nixa-status.sh done"), "{out}");
    assert!(out.contains("Dry run"), "{out}");
    assert!(!out.contains("keep me"), "{out}");
    assert_eq!(fs::read_to_string(&settings).unwrap(), CLAUDE_START);
    assert!(backups(&h.dir.join(".claude")).is_empty());

    let real = nixa(&h, &["hooks", "install", "claude", "--apply"]);
    let out = text(&real.stdout);
    assert_eq!(real.status.code(), Some(0), "{out}{}", text(&real.stderr));
    assert!(out.contains("backup: ") && out.contains("written"), "{out}");
    let after = fs::read_to_string(&settings).unwrap();
    assert!(after.find("zeta").unwrap() < after.find("\"env\"").unwrap());
    assert!(after.find("\"env\"").unwrap() < after.find("alpha").unwrap());
    assert!(after.contains("2.50"));
    let v: Value = serde_json::from_str(&after).unwrap();
    assert_eq!(v["zeta"], "keep me");
    assert_eq!(v["env"]["A"], "1");
    let stop = v["hooks"]["Stop"].as_array().unwrap();
    assert_eq!(stop.len(), 2);
    assert_eq!(stop[0]["hooks"][0]["command"], "say done");
    assert_eq!(v["hooks"].as_object().unwrap().len(), 10);
    assert_eq!(fs::metadata(&settings).unwrap().permissions().mode() & 0o777, 0o644);
    let saved = backups(&h.dir.join(".claude"));
    assert_eq!(saved.len(), 1);
    assert_eq!(fs::read_to_string(&saved[0]).unwrap(), CLAUDE_START);

    let again = nixa(&h, &["hooks", "install", "claude", "--apply"]);
    assert!(text(&again.stdout).contains("no change"), "{}", text(&again.stdout));
    assert_eq!(fs::read_to_string(&settings).unwrap(), after);
    assert_eq!(backups(&h.dir.join(".claude")).len(), 1);

    let status = nixa(&h, &["hooks", "status", "claude"]);
    assert!(text(&status.stdout).contains("installed, 10 of 10 Nixa hooks"), "{}", text(&status.stdout));

    let gone = nixa(&h, &["hooks", "remove", "claude", "--apply"]);
    assert_eq!(gone.status.code(), Some(0), "{}", text(&gone.stderr));
    assert!(text(&gone.stdout).contains("- PermissionRequest"), "{}", text(&gone.stdout));
    let back: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
    let start: Value = serde_json::from_str(CLAUDE_START).unwrap();
    assert_eq!(back, start);
    assert_eq!(backups(&h.dir.join(".claude")).len(), 2);

    let status = nixa(&h, &["--json", "hooks", "status", "claude"]);
    let v: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(v[0]["state"], "not installed");
}

#[test]
fn codex_new_file_and_the_hook_command_runs() {
    let h = home(&[".codex"]);
    let real = nixa(&h, &["hooks", "install", "codex", "--apply"]);
    let out = text(&real.stdout);
    assert_eq!(real.status.code(), Some(0), "{out}{}", text(&real.stderr));
    assert!(!out.contains("backup: "), "{out}");
    assert!(out.contains("trust the new hooks"), "{out}");
    let file = h.dir.join(".codex/hooks.json");
    assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    let v: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    let cmd = v["hooks"]["PermissionRequest"][0]["hooks"][0]["command"].as_str().unwrap().to_string();
    let script = repo_hooks().join("nixa-status.sh");
    assert!(cmd.starts_with(&script.display().to_string()), "{cmd}");

    let log = h.dir.join("called.txt");
    let fake = h.dir.join("fake-nixa");
    fs::write(&fake, format!("#!/bin/sh\nprintf '%s|' \"$@\" > '{}'\n", log.display())).unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    let ran = Command::new("/bin/sh")
        .arg("-c")
        .arg(&cmd)
        .env("NIXA_TERM_ID", "m7")
        .env("NIXA_ENGINE_SOCKET", "/tmp/none.sock")
        .env("TERM_PROGRAM", "nixa")
        .env("NIXA_BIN", &fake)
        .stdin(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(ran.success());
    assert_eq!(fs::read_to_string(&log).unwrap(), "term|status|blocked|needs permission|");

    let gone = nixa(&h, &["hooks", "remove", "codex", "--apply"]);
    assert_eq!(gone.status.code(), Some(0));
    assert_eq!(fs::read_to_string(&file).unwrap(), "{}\n");
}

#[test]
fn omp_link_backup_and_remove() {
    let h = home(&[".omp/agent"]);
    let link = h.dir.join(".omp/agent/extensions/nixa-status.ts");
    let dry = nixa(&h, &["hooks", "install", "omp"]);
    assert!(text(&dry.stdout).contains("+ link "), "{}", text(&dry.stdout));
    assert!(fs::symlink_metadata(&link).is_err());

    let real = nixa(&h, &["hooks", "install", "omp", "--apply"]);
    assert_eq!(real.status.code(), Some(0), "{}", text(&real.stderr));
    assert_eq!(fs::read_link(&link).unwrap(), repo_hooks().join("omp/nixa-status.ts"));
    assert!(text(&nixa(&h, &["hooks", "status", "omp"]).stdout).contains("installed"));
    assert!(text(&nixa(&h, &["hooks", "install", "omp", "--apply"]).stdout).contains("no change"));

    fs::remove_file(&link).unwrap();
    fs::write(&link, "mine").unwrap();
    let left = nixa(&h, &["hooks", "remove", "omp", "--apply"]);
    assert!(text(&left.stdout).contains("not ours"), "{}", text(&left.stdout));
    assert_eq!(fs::read_to_string(&link).unwrap(), "mine");

    let swap = nixa(&h, &["hooks", "install", "omp", "--apply"]);
    assert!(text(&swap.stdout).contains("it moves to"), "{}", text(&swap.stdout));
    let saved = backups(&h.dir.join(".omp/agent/extensions"));
    assert_eq!(saved.len(), 1);
    assert_eq!(fs::read_to_string(&saved[0]).unwrap(), "mine");
    assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());

    let gone = nixa(&h, &["hooks", "remove", "omp", "--apply"]);
    assert_eq!(gone.status.code(), Some(0));
    assert!(fs::symlink_metadata(&link).is_err());
}

#[test]
fn cursor_hooks_are_a_flat_list_with_a_version() {
    let h = home(&[".cursor"]);
    let file = h.dir.join(".cursor/hooks.json");
    fs::write(&file, r#"{"hooks":{"stop":[{"command":"say done"}]}}"#).unwrap();
    let real = nixa(&h, &["hooks", "install", "cursor", "--apply"]);
    let out = text(&real.stdout);
    assert_eq!(real.status.code(), Some(0), "{out}{}", text(&real.stderr));
    assert!(out.contains("restart Cursor"), "{out}");
    let v: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(v["version"], 1);
    assert_eq!(v["hooks"].as_object().unwrap().len(), 8);
    let stop = v["hooks"]["stop"].as_array().unwrap();
    assert_eq!(stop.len(), 2);
    assert_eq!(stop[0]["command"], "say done");
    let script = repo_hooks().join("nixa-status.sh").display().to_string();
    assert_eq!(stop[1]["command"].as_str().unwrap(), format!("{script} done"));
    let start = v["hooks"]["sessionStart"][0]["command"].as_str().unwrap();
    assert_eq!(start, format!("{script} --resume cursor idle"));

    let status = nixa(&h, &["hooks", "status", "cursor"]);
    assert!(text(&status.stdout).contains("installed, 8 of 8 Nixa hooks"), "{}", text(&status.stdout));
    assert!(text(&nixa(&h, &["hooks", "install", "cursor-agent", "--apply"]).stdout).contains("no change"));

    let gone = nixa(&h, &["hooks", "remove", "cursor", "--apply"]);
    assert_eq!(gone.status.code(), Some(0), "{}", text(&gone.stderr));
    let back: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(back, serde_json::json!({"hooks": {"stop": [{"command": "say done"}]}, "version": 1}));
}

#[test]
fn missing_agents_are_skipped_and_nothing_is_made() {
    let h = home(&[]);
    let out = nixa(&h, &["hooks", "install", "--apply"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout).matches("skipped").count(), 4, "{}", text(&out.stdout));
    assert_eq!(fs::read_dir(&h.dir).unwrap().count(), 0);
    let bad = nixa(&h, &["hooks", "install", "hermes"]);
    assert_eq!(bad.status.code(), Some(2));
    let bad = nixa(&h, &["hooks", "status", "--apply"]);
    assert_eq!(bad.status.code(), Some(2));
}

#[test]
fn broken_json_is_left_alone() {
    let h = home(&[".claude", ".codex"]);
    let settings = h.dir.join(".claude/settings.json");
    fs::write(&settings, "{\"a\": 1,").unwrap();
    fs::write(h.dir.join(".codex/hooks.json"), "{\"hooks\": []}").unwrap();
    let out = nixa(&h, &["hooks", "install", "claude", "codex", "--apply"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("is not valid JSON"), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("is not an object"), "{}", text(&out.stderr));
    assert_eq!(fs::read_to_string(&settings).unwrap(), "{\"a\": 1,");
    assert!(backups(&h.dir.join(".claude")).is_empty());
}

#[test]
fn hooks_folder_with_odd_path_and_linked_settings() {
    let h = home(&[".claude", "dotfiles"]);
    let odd = h.dir.join("my hooks'x");
    fs::create_dir_all(odd.join("claude")).unwrap();
    let src = repo_hooks();
    fs::copy(src.join("nixa-status.sh"), odd.join("nixa-status.sh")).unwrap();
    fs::copy(src.join("claude/settings-hooks.json"), odd.join("claude/settings-hooks.json")).unwrap();
    let real = h.dir.join("dotfiles/claude-settings.json");
    fs::write(&real, "{}").unwrap();
    let settings = h.dir.join(".claude/settings.json");
    std::os::unix::fs::symlink(&real, &settings).unwrap();

    let hooks_arg = odd.display().to_string();
    let out = nixa(&h, &["hooks", "install", "claude", "--apply", "--hooks", &hooks_arg]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(fs::symlink_metadata(&settings).unwrap().file_type().is_symlink());
    let v: Value = serde_json::from_str(&fs::read_to_string(&real).unwrap()).unwrap();
    let cmd = v["hooks"]["Stop"][0]["hooks"][0]["command"].as_str().unwrap();
    assert!(cmd.starts_with('\''), "{cmd}");
    let status = nixa(&h, &["hooks", "status", "claude", "--hooks", &hooks_arg]);
    assert!(text(&status.stdout).contains("installed, 10 of 10"), "{}", text(&status.stdout));
    let other = nixa(&h, &["hooks", "status", "claude"]);
    let other = text(&other.stdout);
    assert!(other.contains("not installed") && other.contains("other nixa-status.sh hook"), "{other}");
    assert_eq!(backups(&h.dir.join("dotfiles")).len(), 1);
}

mod session {
    use nixa_protocol::{PROTOCOL_VERSION, Reply, Request, TermId, TermInfo, wire};
    use std::io::{BufReader, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::process::{Command, Output, Stdio};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    type Seen = Arc<Mutex<Vec<Request>>>;

    struct Fake {
        dir: PathBuf,
        socket: PathBuf,
        seen: Seen,
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn term(cmd: &[&str]) -> TermInfo {
        TermInfo {
            id: TermId::from_number(1),
            name: "agent".into(),
            labels: Default::default(),
            parent: None,
            children: vec![],
            cmd: cmd.iter().map(|s| s.to_string()).collect(),
            cwd: "/tmp".into(),
            pid: Some(4242),
            running: true,
            exit_code: None,
            status: None,
            status_note: None,
            keep: false,
            draft: false,
            title: None,
            rows: 24,
            cols: 80,
            created_ms: 0,
            seq: 1,
        }
    }

    fn serve(conn: UnixStream, seen: Seen, info: TermInfo) {
        let Ok(mut writer) = conn.try_clone() else { return };
        let mut reader = BufReader::new(conn);
        while let Ok(Some(req)) = wire::read_request(&mut reader) {
            seen.lock().unwrap().push(req.clone());
            let reply = match &req {
                Request::Ping => Reply::Pong { version: PROTOCOL_VERSION, pid: 1 },
                Request::Get { id } if *id == info.id => Reply::Term { term: info.clone() },
                Request::Get { id } => Reply::not_found(id),
                _ => Reply::Ok,
            };
            if wire::write_reply(&mut writer, &reply).is_err() {
                return;
            }
        }
    }

    fn fake(cmd: &[&str]) -> Fake {
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = PathBuf::from(format!("/tmp/mhs-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("e.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let seen: Seen = Arc::default();
        let s2 = seen.clone();
        let info = term(cmd);
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(conn) = conn else { break };
                let seen = s2.clone();
                let info = info.clone();
                std::thread::spawn(move || serve(conn, seen, info));
            }
        });
        Fake { dir, socket, seen }
    }

    fn run(mut cmd: Command, stdin: &str) -> Output {
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    }

    fn nixa(f: &Fake, args: &[&str], term: Option<&str>, stdin: &str) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_nixa"));
        cmd.arg("--socket").arg(&f.socket).args(args).env("NIXA_DATA_DIR", &f.dir).env_remove("NIXA_TERM_ID").env_remove("NIXA_ENGINE_SOCKET");
        if let Some(t) = term {
            cmd.env("NIXA_TERM_ID", t);
        }
        run(cmd, stdin)
    }

    fn script(f: &Fake, args: &[&str], in_nixa: bool, stdin: &str) -> Output {
        let path = super::repo_hooks().join("nixa-status.sh");
        let mut cmd = Command::new(path);
        cmd.args(args)
            .env("NIXA_TERM_ID", "m1")
            .env("NIXA_ENGINE_SOCKET", &f.socket)
            .env("NIXA_DATA_DIR", &f.dir)
            .env("NIXA_BIN", env!("CARGO_BIN_EXE_nixa"))
            .env("TERM_PROGRAM", if in_nixa { "nixa" } else { "other" });
        run(cmd, stdin)
    }

    fn labels(f: &Fake) -> Vec<(String, Option<String>)> {
        f.seen
            .lock()
            .unwrap()
            .iter()
            .filter_map(|r| match r {
                Request::LabelSet { key, value, .. } => Some((key.clone(), value.clone())),
                _ => None,
            })
            .collect()
    }

    fn statuses(f: &Fake) -> usize {
        f.seen.lock().unwrap().iter().filter(|r| format!("{r:?}").contains("Status")).count()
    }

    fn text(b: &[u8]) -> String {
        String::from_utf8_lossy(b).to_string()
    }

    const START: &str = r#"{"session_id":"7f3c2a10-1b2c-4d5e-9f00-aa11bb22cc33","hook_event_name":"SessionStart","source":"startup","permission_mode":"default"}"#;

    #[test]
    fn session_reads_the_hook_json_and_sets_both_labels() {
        let f = fake(&["claude"]);
        let o = nixa(&f, &["hooks", "session", "claude"], Some("m1"), START);
        assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
        assert_eq!(text(&o.stdout), "m1 resume: claude --resume 7f3c2a10-1b2c-4d5e-9f00-aa11bb22cc33 --dangerously-skip-permissions\n");
        assert_eq!(
            labels(&f),
            vec![
                ("resume.pid".to_string(), Some("4242".to_string())),
                ("resume".to_string(), Some("claude --resume 7f3c2a10-1b2c-4d5e-9f00-aa11bb22cc33 --dangerously-skip-permissions".to_string())),
            ]
        );
    }

    #[test]
    fn session_keeps_the_checks_off_flag_of_the_start() {
        let f = fake(&["codex", "--dangerously-bypass-approvals-and-sandbox"]);
        let o = nixa(&f, &["--json", "hooks", "session", "codex"], Some("m1"), r#"{"session_id":"019a-77"}"#);
        assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
        let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
        assert_eq!(v["command"], "codex resume 019a-77 --dangerously-bypass-approvals-and-sandbox");
        let f = fake(&["claude"]);
        let o = nixa(&f, &["hooks", "session", "claude"], Some("m1"), r#"{"session_id":"abc","permission_mode":"bypassPermissions"}"#);
        assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
        assert_eq!(labels(&f).last().unwrap().1.as_deref(), Some("claude --resume abc --dangerously-skip-permissions"));
        let f = fake(&["omp"]);
        let o = nixa(&f, &["hooks", "session", "omp", "--id", "f00d"], Some("m1"), "");
        assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
        assert_eq!(labels(&f).last().unwrap().1.as_deref(), Some("omp --resume=f00d --auto-approve"));
    }

    #[test]
    fn session_refuses_bad_input_and_writes_nothing() {
        let f = fake(&["claude"]);
        let o = nixa(&f, &["hooks", "session", "claude"], None, START);
        assert_eq!(o.status.code(), Some(2), "{}", text(&o.stderr));
        let o = nixa(&f, &["hooks", "session", "claude"], Some("m1"), r#"{"hook_event_name":"SessionStart"}"#);
        assert_eq!(o.status.code(), Some(2), "{}", text(&o.stderr));
        let o = nixa(&f, &["hooks", "session", "claude"], Some("m1"), r#"{"session_id":"a b; rm -rf ~"}"#);
        assert_eq!(o.status.code(), Some(1), "{}", text(&o.stderr));
        let o = nixa(&f, &["hooks", "session", "hermes", "--id", "x"], Some("m1"), "");
        assert_eq!(o.status.code(), Some(2), "{}", text(&o.stderr));
        let o = nixa(&f, &["hooks", "session", "claude"], Some("m9"), START);
        assert_eq!(o.status.code(), Some(1), "{}", text(&o.stderr));
        assert!(labels(&f).is_empty());
    }

    #[test]
    fn status_script_sets_resume_then_status() {
        let f = fake(&["claude"]);
        let o = script(&f, &["--resume", "claude", "idle"], true, START);
        assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
        assert_eq!(labels(&f).len(), 2);
        assert_eq!(labels(&f)[1].1.as_deref(), Some("claude --resume 7f3c2a10-1b2c-4d5e-9f00-aa11bb22cc33 --dangerously-skip-permissions"));
        assert_eq!(statuses(&f), 1);
        let o = script(&f, &["working"], true, START);
        assert_eq!(o.status.code(), Some(0));
        assert_eq!(labels(&f).len(), 2);
        assert_eq!(statuses(&f), 2);
        let o = script(&f, &["--resume", "claude", "idle"], true, "not json");
        assert_eq!(o.status.code(), Some(0));
        assert_eq!(labels(&f).len(), 2);
        assert_eq!(statuses(&f), 3);
        let o = script(&f, &["--resume"], true, START);
        assert_eq!(o.status.code(), Some(0));
        let o = script(&f, &["--resume", "claude", "idle"], false, START);
        assert_eq!(o.status.code(), Some(0));
        assert_eq!(labels(&f).len(), 2);
        assert_eq!(statuses(&f), 3);
    }

    #[test]
    fn templates_save_the_session_on_start_and_prompt() {
        for (rel, agent) in [("claude/settings-hooks.json", "claude"), ("codex/hooks.json", "codex")] {
            let text = std::fs::read_to_string(super::repo_hooks().join(rel)).unwrap();
            let v: serde_json::Value = serde_json::from_str(&text).unwrap();
            for event in ["SessionStart", "UserPromptSubmit"] {
                let cmd = v["hooks"][event][0]["hooks"][0]["command"].as_str().unwrap();
                assert!(cmd.starts_with(&format!("./hooks/nixa-status.sh --resume {agent} ")), "{rel} {event}: {cmd}");
            }
        }
        let text = std::fs::read_to_string(super::repo_hooks().join("cursor/hooks.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        for event in ["sessionStart", "beforeSubmitPrompt"] {
            let cmd = v["hooks"][event][0]["command"].as_str().unwrap();
            assert!(cmd.starts_with("./hooks/nixa-status.sh --resume cursor "), "cursor {event}: {cmd}");
        }
        let omp = std::fs::read_to_string(super::repo_hooks().join("omp/nixa-status.ts")).unwrap();
        assert!(omp.contains(r#"["hooks", "session", "omp", "--id", id]"#));
    }
}
