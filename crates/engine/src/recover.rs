use crate::engine::Engine;
use crate::handlers;
use crate::terminal;
use nixa_protocol::{CreateSpec, Reply, TermInfo};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

pub const FILE: &str = "open-terminals.json";
const RESUME_LABEL: &str = "resume";
const RESUME_PID_LABEL: &str = "resume.pid";
const EVERY: Duration = Duration::from_secs(2);
const FULL_SAVE_WAKES: u32 = 15;

static STOPPING: AtomicBool = AtomicBool::new(false);

pub fn list_path(socket: &Path) -> PathBuf {
    socket.with_file_name(FILE)
}

pub fn open_list(engine: &Engine) -> Vec<TermInfo> {
    let mut list: Vec<TermInfo> = engine.terms().iter().map(|t| terminal::info(t)).filter(|i| i.running).collect();
    list.sort_by_key(|i| i.created_ms);
    list
}

pub fn save(engine: &Engine) -> io::Result<()> {
    if STOPPING.load(Ordering::SeqCst) {
        return Ok(());
    }
    let path = list_path(&engine.socket);
    let tmp = path.with_extension("tmp");
    let body = serde_json::to_vec(&open_list(engine)).map_err(io::Error::other)?;
    let mut file = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    file.write_all(&body)?;
    file.sync_all()?;
    fs::rename(&tmp, &path)
}

pub fn forget(socket: &Path) {
    STOPPING.store(true, Ordering::SeqCst);
    let _ = fs::remove_file(list_path(socket));
}

pub fn start_saver(engine: Arc<Engine>) {
    let started = thread::Builder::new().name("terminal-list".to_string()).spawn(move || {
        let mut seen = None;
        let mut wakes = 0;
        loop {
            let now = engine.tick();
            wakes += 1;
            if seen != Some(now) || wakes >= FULL_SAVE_WAKES {
                seen = Some(now);
                wakes = 0;
                if let Err(e) = save(&engine) {
                    eprintln!("nixa-engine: cannot save the open terminal list: {e}");
                }
            }
            engine.wait_change(now, Instant::now() + EVERY);
            thread::sleep(Duration::from_millis(300));
        }
    });
    if let Err(e) = started {
        eprintln!("nixa-engine: cannot start the terminal list thread: {e}");
    }
}

fn safe_word(word: &str) -> bool {
    !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || "-_.:/=@".contains(c))
}

pub fn resume_cmd(info: &TermInfo) -> Option<Vec<String>> {
    let label = info.labels.get(RESUME_LABEL)?;
    let first = info.cmd.first()?;
    let program = Path::new(first).file_name()?.to_str()?;
    let mut words: Vec<String> = label.split_whitespace().map(String::from).collect();
    if words.len() < 2 || !words.iter().all(|w| safe_word(w)) || (words[0] != program && words[0] != *first) {
        return None;
    }
    words[0] = first.clone();
    Some(words)
}

pub fn spec_for(info: &TermInfo, parent_open: bool) -> CreateSpec {
    let mut labels = info.labels.clone();
    labels.remove(RESUME_PID_LABEL);
    CreateSpec {
        cmd: resume_cmd(info).unwrap_or_else(|| info.cmd.clone()),
        cwd: Some(info.cwd.clone()),
        name: Some(info.name.clone()),
        labels,
        parent: info.parent.clone().filter(|_| parent_open),
        env: Default::default(),
        rows: Some(info.rows),
        cols: Some(info.cols),
        keep: info.keep,
    }
}

pub fn restore(engine: &Arc<Engine>) -> usize {
    let Ok(body) = fs::read(list_path(&engine.socket)) else {
        return 0;
    };
    let list: Vec<TermInfo> = serde_json::from_slice(&body).unwrap_or_default();
    let mut back = 0;
    for info in list {
        if engine.get(&info.id).is_some() {
            continue;
        }
        let parent_open = info.parent.as_ref().is_some_and(|p| engine.get(p).is_some());
        let mut spec = spec_for(&info, parent_open);
        if !Path::new(&info.cwd).is_dir() {
            spec.cwd = None;
        }
        match handlers::create_as(engine, spec, Some(info.id.clone())) {
            Reply::Term { .. } => back += 1,
            other => eprintln!("nixa-engine: cannot bring back {}: {other:?}", info.id),
        }
    }
    back
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(cmd: &[&str], resume: Option<&str>) -> TermInfo {
        let mut labels = std::collections::BTreeMap::new();
        if let Some(r) = resume {
            labels.insert(RESUME_LABEL.to_string(), r.to_string());
            labels.insert(RESUME_PID_LABEL.to_string(), "41".to_string());
        }
        TermInfo {
            id: "web-claude-3".parse().unwrap(),
            name: "Claude".into(),
            labels,
            parent: None,
            children: Vec::new(),
            cmd: cmd.iter().map(|s| s.to_string()).collect(),
            cwd: "/tmp".into(),
            pid: None,
            running: true,
            exit_code: None,
            status: None,
            status_note: None,
            keep: false,
            draft: false,
            title: None,
            rows: 30,
            cols: 100,
            created_ms: 0,
            seq: 0,
        }
    }

    #[test]
    fn agents_come_back_with_their_resume_command() {
        let t = info(&["/usr/local/bin/claude", "--model", "x"], Some("claude --resume abc-1"));
        assert_eq!(resume_cmd(&t).unwrap(), vec!["/usr/local/bin/claude", "--resume", "abc-1"]);
        let spec = spec_for(&t, false);
        assert!(!spec.labels.contains_key(RESUME_PID_LABEL));
        assert_eq!(spec.rows, Some(30));
    }

    #[test]
    fn a_strange_resume_label_keeps_the_first_command() {
        assert!(resume_cmd(&info(&["claude"], Some("claude --resume a; rm -rf x"))).is_none());
        assert!(resume_cmd(&info(&["claude"], Some("codex resume a"))).is_none());
        assert!(resume_cmd(&info(&["zsh"], None)).is_none());
        assert_eq!(spec_for(&info(&["zsh"], None), true).cmd, vec!["zsh"]);
    }
}
