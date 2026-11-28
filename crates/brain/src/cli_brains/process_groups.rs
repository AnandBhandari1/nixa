use crate::app::lock;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::process::{Child, Command};

const GRACE: Duration = Duration::from_millis(1500);
const POLL: Duration = Duration::from_millis(25);

#[derive(Clone, Default)]
pub(super) struct Groups(Arc<Mutex<Registry>>);

#[derive(Default)]
struct Registry {
    closing: bool,
    next: u64,
    groups: HashMap<i32, (u64, bool)>,
    programs: HashMap<i32, String>,
    file: Option<PathBuf>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Started {
    pid: i32,
    program: String,
}

#[derive(Clone)]
pub(super) struct Group {
    groups: Groups,
    pid: i32,
    serial: u64,
}

impl Groups {
    pub(super) fn open(file: PathBuf) -> Groups {
        let left: Vec<Started> = crate::util::read_json(&file).unwrap_or_default();
        if !left.is_empty() {
            std::thread::spawn(move || stop_orphans(left));
        }
        let groups = Groups::default();
        lock(&groups.0).file = Some(file);
        groups.save();
        groups
    }

    fn save(&self) {
        let registry = lock(&self.0);
        let Some(file) = registry.file.clone() else { return };
        let list: Vec<Started> = registry.groups.keys().filter_map(|pid| registry.programs.get(pid).map(|program| Started { pid: *pid, program: program.clone() })).collect();
        drop(registry);
        if let Err(e) = crate::util::write_json(&file, &list) {
            crate::util::log("cli-brain", &format!("the started process list was not written: {e}"));
        }
    }

    pub(super) fn spawn(&self, command: &mut Command) -> io::Result<(Child, Group)> {
        let mut registry = lock(&self.0);
        if registry.closing {
            return Err(io::Error::other("Nixa is stopping"));
        }
        let child = command.process_group(0).spawn()?;
        let pid = child
            .id()
            .ok_or_else(|| io::Error::other("CLI process has no PID"))? as i32;
        registry.next += 1;
        let serial = registry.next;
        registry.groups.insert(pid, (serial, false));
        let program = command.as_std().get_program().to_string_lossy().to_string();
        registry.programs.retain(|known, _| signal(*known, 0));
        registry.programs.insert(pid, program);
        drop(registry);
        self.save();
        Ok((
            child,
            Group {
                groups: self.clone(),
                pid,
                serial,
            },
        ))
    }

    pub(super) async fn shutdown(&self) {
        {
            let mut registry = lock(&self.0);
            registry.closing = true;
            registry.groups.retain(|pid, (_, stopping)| {
                *stopping = true;
                signal(*pid, libc::SIGTERM)
            });
        }

        let deadline = tokio::time::Instant::now() + GRACE;
        loop {
            {
                let mut registry = lock(&self.0);
                registry.groups.retain(|pid, _| signal(*pid, 0));
                if registry.groups.is_empty() {
                    return;
                }
                if tokio::time::Instant::now() >= deadline {
                    for (pid, _) in registry.groups.drain() {
                        signal(pid, libc::SIGKILL);
                    }
                    return;
                }
            }
            tokio::time::sleep(POLL).await;
        }
    }
}

impl Group {
    pub(super) fn stop(&self) {
        {
            let mut registry = lock(&self.groups.0);
            if registry.groups.get(&self.pid) != Some(&(self.serial, false)) {
                return;
            }
            if !signal(self.pid, libc::SIGTERM) {
                registry.groups.remove(&self.pid);
                return;
            }
            registry.groups.insert(self.pid, (self.serial, true));
        }

        let groups = self.groups.clone();
        let pid = self.pid;
        let serial = self.serial;
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                tokio::time::sleep(GRACE).await;
                kill_current(&groups, pid, serial);
            });
        } else {
            kill_current(&groups, pid, serial);
        }
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        self.stop();
    }
}

fn kill_current(groups: &Groups, pid: i32, serial: u64) {
    let mut registry = lock(&groups.0);
    if registry.groups.get(&pid) == Some(&(serial, true)) {
        signal(pid, libc::SIGKILL);
        registry.groups.remove(&pid);
    }
}

fn name_of(path: &str) -> &str {
    Path::new(path.trim()).file_name().and_then(|n| n.to_str()).unwrap_or("")
}

pub(super) fn is_orphan(ps_line: &str, program: &str) -> bool {
    let line = ps_line.trim();
    let Some((ppid, comm)) = line.split_once(char::is_whitespace) else { return false };
    ppid.trim() == "1" && !name_of(program).is_empty() && name_of(comm) == name_of(program)
}

fn stop_orphans(left: Vec<Started>) {
    let mut stopped = Vec::new();
    for item in left {
        let Ok(out) = std::process::Command::new("/bin/ps").args(["-o", "ppid=,comm=", "-p", &item.pid.to_string()]).output() else { continue };
        if is_orphan(&String::from_utf8_lossy(&out.stdout), &item.program) && signal(item.pid, libc::SIGTERM) {
            crate::util::log("cli-brain", &format!("stopped a {} left by an earlier brain (pid {})", name_of(&item.program), item.pid));
            stopped.push(item.pid);
        }
    }
    if stopped.is_empty() {
        return;
    }
    std::thread::sleep(GRACE);
    for pid in stopped {
        signal(pid, libc::SIGKILL);
    }
}

fn signal(group: i32, sig: i32) -> bool {
    if group <= 0 {
        return false;
    }
    unsafe { libc::killpg(group, sig) == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, BufReader};

    #[test]
    fn knows_an_orphan_only_by_parent_and_name() {
        assert!(is_orphan("    1 /Users/x/.opencode/bin/opencode", "/Users/x/.opencode/bin/opencode"));
        assert!(!is_orphan("  512 /Users/x/.opencode/bin/opencode", "/Users/x/.opencode/bin/opencode"));
        assert!(!is_orphan("    1 /usr/bin/python3", "/Users/x/.opencode/bin/opencode"));
        assert!(!is_orphan("", "opencode"));
    }

    #[tokio::test]
    async fn open_writes_the_list_of_started_groups() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("started.json");
        let groups = Groups::open(file.clone());
        let (_child, _group) = groups.spawn(Command::new("/bin/sleep").arg("5").kill_on_drop(true)).unwrap();
        let list: Vec<Started> = crate::util::read_json(&file).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].program, "/bin/sleep");
    }

    #[tokio::test]
    async fn shutdown_stops_the_group_and_refuses_new_children() {
        let groups = Groups::default();
        let mut shell = Command::new("/bin/sh");
        shell
            .args(["-c", "trap '' TERM; /bin/sleep 30 & echo $!; wait"])
            .stdout(Stdio::piped())
            .kill_on_drop(true);
        let (mut child, group) = groups.spawn(&mut shell).unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .await
            .unwrap();
        let descendant = line.trim().parse::<i32>().unwrap();
        assert_eq!(unsafe { libc::getpgid(descendant) }, group.pid);

        groups.shutdown().await;
        let status = tokio::time::timeout(Duration::from_secs(2), child.wait())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.signal(), Some(libc::SIGKILL));
        assert!(groups.spawn(&mut Command::new("/usr/bin/true")).is_err());
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let stat = std::process::Command::new("/bin/ps")
                .args(["-o", "stat=", "-p", line.trim()])
                .output()
                .unwrap();
            if !stat.status.success()
                || String::from_utf8_lossy(&stat.stdout)
                    .trim_start()
                    .starts_with('Z')
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "CLI descendant still runs"
            );
            tokio::time::sleep(POLL).await;
        }
    }
}
