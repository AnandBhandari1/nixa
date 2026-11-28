use super::outcome::BUILD_LOG;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

pub const APP_DEST: &str = "/Applications/Nixa.app";
const BUILD_ARGS: [&str; 7] = ["run", "-p", "xtask", "--", "bundle", "--no-live-check", "--out"];
const BUILD_MS: u64 = 60 * 60 * 1000;
const DROP_ENV: [&str; 6] = ["NIXA_DATA_DIR", "NIXA_ENGINE_SOCKET", "NIXA_BRAIN_ADDR", "NIXA_TERM_ID", "NIXA_BIN", "CARGO_TARGET_DIR"];
const FAIL_LINES: usize = 6;

pub fn expand(text: &str, home: Option<&Path>) -> PathBuf {
    let text = text.trim();
    match (text.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(text),
    }
}

pub fn is_repo(dir: &Path) -> bool {
    dir.is_absolute() && dir.join("Cargo.lock").is_file() && dir.join("crates/xtask/Cargo.toml").is_file() && dir.join("crates/app/Cargo.toml").is_file()
}

pub fn find_cargo(path: &str) -> Option<PathBuf> {
    path.split(':').filter(|d| d.starts_with('/')).map(|d| Path::new(d).join("cargo")).find(|f| f.is_file())
}

pub fn alive(pid: u32) -> bool {
    pid > 1 && Command::new("/bin/kill").args(["-0", &pid.to_string()]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn last_lines(text: &str, count: usize) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let start = lines.len().saturating_sub(count);
    crate::util::clip(&lines[start..].join(" | "), 600)
}

fn build_command(cargo: &Path, path: &str, repo: &Path, output: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(cargo);
    command.args(BUILD_ARGS).arg(output).current_dir(repo).env("PATH", path).stdin(Stdio::null()).kill_on_drop(true);
    for name in DROP_ENV {
        command.env_remove(name);
    }
    command
}

pub async fn build(repo: &Path, output: &Path) -> Result<(), String> {
    let log = output.join(BUILD_LOG);
    let out = std::fs::File::create(&log).map_err(|e| format!("cannot write {}: {e}", log.display()))?;
    let err = out.try_clone().map_err(|e| format!("cannot write {}: {e}", log.display()))?;
    let path = nixa_protocol::paths::tool_path(&std::env::var("PATH").unwrap_or_default(), nixa_protocol::paths::home_dir().as_deref());
    let cargo = find_cargo(&path).ok_or("cargo was not found on PATH or in ~/.cargo/bin")?;
    let mut command = build_command(&cargo, &path, repo, output);
    let mut child = command.stdout(out).stderr(err).spawn().map_err(|e| format!("cargo did not start: {e}"))?;
    match tokio::time::timeout(Duration::from_millis(BUILD_MS), child.wait()).await {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(status)) => {
            let tail = crate::manage::diagnostics::tail(&log, 16_000);
            Err(format!("the build failed (exit {}): {}. The whole log is {}", status.code().unwrap_or(-1), last_lines(&tail, FAIL_LINES), log.display()))
        }
        Ok(Err(e)) => Err(format!("the build did not finish: {e}")),
        Err(_) => {
            let _ = child.kill().await;
            Err(format!("the build took longer than {} minutes and was stopped", BUILD_MS / 60_000))
        }
    }
}

fn release_command(cargo: &Path, repo: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(cargo);
    command.args(["run", "-p", "xtask", "--", "release"]).current_dir(repo).stdin(Stdio::null());
    command.env_remove("NIXA_TERM_ID");
    command
}

pub async fn activate(app: &crate::app::App, repo: &Path, bundle: &Path, dir: &Path) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    let path = nixa_protocol::paths::tool_path(&std::env::var("PATH").unwrap_or_default(), nixa_protocol::paths::home_dir().as_deref());
    let cargo = find_cargo(&path).ok_or("cargo was not found")?;
    let mut inspect = release_command(&cargo, repo);
    inspect.arg("inspect").arg(bundle).kill_on_drop(true);
    let inspected = tokio::time::timeout(Duration::from_secs(600), inspect.output()).await
        .map_err(|_| "candidate inspection timed out")?.map_err(|e| e.to_string())?;
    if !inspected.status.success() {
        return Err(format!("Candidate verification failed: {}", last_lines(&String::from_utf8_lossy(&inspected.stderr), FAIL_LINES)));
    }
    let text = String::from_utf8_lossy(&inspected.stdout);
    let digest = text.lines().find_map(|line| line.strip_prefix("SHA-256: ")).filter(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())).ok_or("the candidate inspector did not return a fingerprint")?;
    if dir.join("cancel").exists() || !crate::settings_api::nixa_on(&app.dir) || crate::app::lock(&app.brain).cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("Self update was stopped before activation".into());
    }
    let log = std::fs::File::create(dir.join(super::outcome::RUNNER_LOG)).map_err(|e| e.to_string())?;
    let mut command = release_command(&cargo, repo);
    command.arg("automatic").arg(bundle).arg(digest).arg(std::process::id().to_string())
        .arg(&app.dir).arg(nixa_protocol::paths::brain_addr()).arg(nixa_protocol::paths::socket_path())
        .stdout(log.try_clone().map_err(|e| e.to_string())?).stderr(log);
    command.as_std_mut().process_group(0);
    let child = command.spawn().map_err(|e| format!("could not start the verified release runner: {e}"))?;
    std::fs::write(dir.join(super::outcome::PID_FILE), child.id().ok_or("release runner has no identity")?.to_string()).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
#[tokio::test]
async fn staging_passes_an_isolated_output_without_live_checks() {
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join("candidate with spaces");
    let command = build_command(Path::new("/usr/bin/printf"), "/usr/bin", root.path(), &out);
    let args = command.as_std().get_args().collect::<Vec<_>>();
    assert!(args.ends_with(&["--no-live-check".as_ref(), "--out".as_ref(), out.as_os_str()]));
    assert!(command.as_std().get_envs().any(|(key, value)| key == "NIXA_ENGINE_SOCKET" && value.is_none()));
    let result = build_command(Path::new("/bin/echo"), "/bin", root.path(), &out).output().await.unwrap();
    assert!(result.status.success());
    assert!(String::from_utf8(result.stdout).unwrap().contains(&*out.to_string_lossy()));
}
