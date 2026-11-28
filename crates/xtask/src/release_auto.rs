use super::{Children, DEST, Runtime, activate, candidate, digest_arg, nonce, resolve};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

fn descends_from(mut pid: u32, parent: u32, rows: &HashMap<u32, u32>) -> bool {
    for _ in 0..256 {
        let Some(next) = rows.get(&pid).copied() else { return false };
        if next == parent {
            return parent > 1;
        }
        if next <= 1 || next == pid {
            return false;
        }
        pid = next;
    }
    false
}

fn brain_caller(pid: u32, installed: &Path) -> Result<(), String> {
    let expected = crate::layout::macos_dir(installed).join("nixa-app");
    if pid <= 1 || crate::release_runtime::process_path(pid as i32).as_deref() != Some(&expected) {
        return Err("automatic activation is available only to the installed Nixa brain".into());
    }
    let output = Command::new("/bin/ps").args(["-A", "-o", "pid=,ppid="]).output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("could not verify the brain caller ancestry".into());
    }
    let rows = String::from_utf8_lossy(&output.stdout).lines().filter_map(|line| {
        let mut words = line.split_whitespace();
        Some((words.next()?.parse().ok()?, words.next()?.parse().ok()?))
    }).collect();
    if !descends_from(std::process::id(), pid, &rows) {
        return Err("automatic activation must be started by the installed Nixa brain".into());
    }
    Ok(())
}

pub(super) fn run(args: &[String]) -> Result<(), String> {
    let [_, _, pid, data, _, _] = args else {
        return Err("automatic activation needs a candidate, digest and its brain runtime identity".into());
    };
    brain_caller(pid.parse().map_err(|_| "invalid brain identity")?, Path::new(DEST))?;
    let result = perform(args);
    let journal = Path::new(data).join("go-live");
    let text = match &result {
        Ok(()) => "Verified automatic activation completed on the original engine.".to_string(),
        Err(error) => format!("NOT LIVE: {error}"),
    };
    let _ = std::fs::write(journal.join("steps.log"), text);
    let _ = std::fs::write(journal.join("end"), if result.is_ok() { "ok\n" } else { "failed\n" });
    result
}

fn perform(args: &[String]) -> Result<(), String> {
    let [bundle, digest, pid, data, addr, socket] = args else {
        return Err("automatic activation needs a candidate, digest and its brain runtime identity".into());
    };
    let pid = pid.parse::<u32>().map_err(|_| "invalid brain identity")?;
    let installed = Path::new(DEST);
    brain_caller(pid, installed)?;
    digest_arg(digest)?;
    let src = resolve(bundle)?;
    if candidate(&src)? != *digest {
        return Err("candidate changed since inspection".into());
    }
    let previous = candidate(installed)?;
    let runtime = Runtime {
        data: data.into(), socket: socket.into(),
        addr: addr.parse().map_err(|_| "invalid brain address")?,
        wait: Duration::from_secs(25),
    };
    let journal = runtime.data.join("go-live");
    if super::read(&journal)?.is_some_and(|record| !record.phase.complete()) {
        return recover_first(installed, &journal, &runtime, pid);
    }
    if previous == *digest {
        return Err("this exact candidate is already installed".into());
    }
    nixa_protocol::paths::ensure_private_dir(&journal).map_err(|e| e.to_string())?;
    let log = OpenOptions::new().create(true).append(true).mode(0o600).open(journal.join("automatic.log")).map_err(|e| e.to_string())?;
    let engine = runtime.engine_pid()?;
    let scratch = tempfile::Builder::new().prefix("nixa-auto-preflight-").tempdir().map_err(|e| e.to_string())?;
    crate::release_runtime::preflight(&src, &nonce()?, &scratch.path().join("data"), &log)?;
    if runtime.data.join("off").exists() || journal.join("cancel").exists() {
        return Err("Nixa is off; automatic activation was cancelled".into());
    }
    brain_caller(pid, installed)?;
    runtime.retire(installed, pid, engine)?;
    let mut children = Children::default();
    let result = activate(&src, digest, installed, &journal, &runtime, |_, _| Ok(()), &mut children);
    if result.is_err() && children.empty() && candidate(installed).ok().as_deref() == Some(&previous) {
        runtime.launch(installed, &nonce()?, engine, true, &log, &mut children)?;
    }
    result
}

fn recover_first(installed: &Path, journal: &Path, runtime: &Runtime, pid: u32) -> Result<(), String> {
    let record = super::read(journal)?.ok_or("no activation to recover")?;
    let engine = runtime.engine_pid()?;
    brain_caller(pid, installed)?;
    runtime.retire(installed, pid, engine)?;
    let mut children = Children::default();
    let result = super::recover(installed, journal, runtime, &record.old, |_| Ok(()), &mut children);
    if result.is_err() && children.empty() {
        let log = OpenOptions::new().create(true).append(true).mode(0o600).open(journal.join("automatic.log")).map_err(|e| e.to_string())?;
        runtime.launch(installed, &nonce()?, engine, true, &log, &mut children)?;
    }
    result?;
    Err("the previous activation was incomplete, so Nixa recovered the previous release first; run the self update again".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_actual_brain_ancestors_authorize_automatic_activation() {
        let rows = HashMap::from([(10, 9), (9, 8), (8, 1), (20, 20)]);
        assert!(descends_from(10, 8, &rows));
        assert!(!descends_from(10, 20, &rows));
        assert!(!descends_from(10, 1, &rows));
        assert!(!descends_from(20, 8, &rows));
        assert!(brain_caller(std::process::id(), Path::new("/missing/Nixa.app")).is_err());
    }
}
