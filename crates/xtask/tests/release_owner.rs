#![cfg(target_os = "macos")]

use nixa_protocol::{Reply, Request, TermInfo, wire};
use serde_json::json;
use std::fs::{self, File};
use std::io::BufReader;
use std::os::fd::FromRawFd;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::{Command, Stdio};

fn terminal(pid: u32) -> TermInfo {
    serde_json::from_value(json!({
        "id": "m1", "name": "test agent", "cwd": "/tmp", "pid": pid,
        "running": true, "rows": 24, "cols": 80, "created_ms": 0
    }))
    .unwrap()
}

fn pty() -> (File, File) {
    let (mut master, mut slave) = (-1, -1);
    let result = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
    unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) }
}

fn boundary(home: &Path, pid: u32, action: &str, claimed: Option<&str>) -> String {
    let data = home.join("Library/Application Support/Nixa");
    fs::create_dir_all(&data).unwrap();
    let socket = data.join("engine.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let engine = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        assert!(matches!(wire::read_request(&mut reader).unwrap(), Some(Request::List { .. })));
        wire::write_reply(&mut &stream, &Reply::Terms { terms: vec![terminal(pid)] }).unwrap();
    });
    let (_master, slave) = pty();
    let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
    command
        .args(["release", action])
        .env("HOME", home)
        .env("NIXA_DATA_DIR", "/tmp/nx-mt")
        .env("NIXA_BRAIN_ADDR", "127.0.0.1:7475")
        .env("NIXA_ENGINE_SOCKET", "/tmp/nx-mt/e.sock")
        .env_remove("NIXA_TERM_ID")
        .env_remove("NIXA_AGENT")
        .stdin(slave.try_clone().unwrap())
        .stdout(slave)
        .stderr(Stdio::piped());
    if let Some(id) = claimed {
        command.env("NIXA_TERM_ID", id);
    }
    if action == "activate" {
        command.arg(home.join("missing-candidate.app")).arg("a".repeat(64));
    } else {
        command.arg("invalid-digest");
    }
    let output = command.output().unwrap();
    engine.join().unwrap();
    fs::remove_file(socket).unwrap();
    assert!(!output.status.success());
    assert!(!data.join("go-live").exists());
    String::from_utf8(output.stderr).unwrap()
}

fn scratch() -> tempfile::TempDir {
    fs::create_dir_all("/tmp/nx-mt").unwrap();
    tempfile::Builder::new().prefix("owner-").tempdir_in("/tmp/nx-mt").unwrap()
}

#[test]
fn automatic_release_refuses_a_forged_brain_before_touching_scratch_inputs() {
    let root = scratch();
    let candidate = root.path().join("candidate.app");
    let data = root.path().join("data");
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["release", "automatic"])
        .arg(&candidate).arg("a".repeat(64)).arg(std::process::id().to_string())
        .arg(&data).arg("127.0.0.1:7475").arg(root.path().join("e.sock"))
        .env("NIXA_DATA_DIR", "/tmp/nx-mt")
        .env("NIXA_BRAIN_ADDR", "127.0.0.1:7475")
        .env("NIXA_ENGINE_SOCKET", "/tmp/nx-mt/e.sock")
        .output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr).unwrap().contains("only to the installed Nixa brain"));
    assert!(!data.exists());
    assert!(!candidate.exists());
}

#[test]
fn agent_ancestry_is_refused_at_public_activate_and_recover_boundary() {
    let home = scratch();
    for action in ["activate", "recover"] {
        for claimed in [None, Some("m1")] {
            let error = boundary(home.path(), std::process::id(), action, claimed);
            assert!(error.contains("Terminal m1 does not activate or recover a release by hand"), "{error}");
            assert!(error.contains("Nothing was changed"), "{error}");
            assert!(!error.contains("Only the user"), "{error}");
        }
        let error = boundary(home.path(), std::process::id(), action, Some("m9"));
        assert!(error.contains("not an open Nixa terminal"), "{error}");
    }
}

#[test]
fn outside_owner_passes_guard_without_activating_invalid_scratch_inputs() {
    let home = scratch();
    let mut sibling = Command::new("/bin/sleep").arg("60").spawn().unwrap();
    let activate = boundary(home.path(), sibling.id(), "activate", None);
    let recover = boundary(home.path(), sibling.id(), "recover", None);
    sibling.kill().unwrap();
    sibling.wait().unwrap();
    assert!(activate.contains("the staged path must be a real .app directory"), "{activate}");
    assert!(recover.contains("use the full lowercase SHA-256"), "{recover}");
    for error in [activate, recover] {
        assert!(!error.contains("Only the user"), "{error}");
        assert!(!error.contains("requires a separate interactive terminal"), "{error}");
    }
}
