use crate::engine::Engine;
use nixa_protocol::{ProcInfo, Reply, TermProcs};

const AGENTS: &[&str] = &["claude", "codex", "agy", "omp", "gemini", "cursor-agent", "opencode", "kimi", "crush", "aider", "goose", "droid", "amp", "copilot"];

pub fn is_agent(name: &str) -> bool {
    AGENTS.contains(&name)
}

pub fn parse_ps(text: &str) -> Vec<ProcInfo> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (pid, rest) = line.split_once(char::is_whitespace)?;
            let (ppid, comm) = rest.trim_start().split_once(char::is_whitespace)?;
            let comm = comm.trim();
            let name = comm.rsplit('/').next().unwrap_or(comm).to_string();
            Some(ProcInfo { pid: pid.parse().ok()?, ppid: ppid.parse().ok()?, agent: is_agent(&name), name })
        })
        .collect()
}

pub fn below(rows: &[ProcInfo], root: u32) -> Vec<ProcInfo> {
    let mut out = Vec::new();
    let mut todo = vec![root];
    while let Some(parent) = todo.pop() {
        for row in rows.iter().filter(|r| r.ppid == parent && r.pid != root) {
            if out.iter().any(|o: &ProcInfo| o.pid == row.pid) {
                continue;
            }
            todo.push(row.pid);
            out.push(row.clone());
        }
    }
    out.sort_by_key(|p| p.pid);
    out
}

fn ps_rows() -> Vec<ProcInfo> {
    std::process::Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,comm="])
        .output()
        .map(|o| parse_ps(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

pub fn list(engine: &Engine) -> Reply {
    let rows = ps_rows();
    let terms = engine
        .terms()
        .iter()
        .map(|t| crate::terminal::info(t))
        .filter(|t| t.running)
        .filter_map(|t| t.pid.map(|pid| TermProcs { id: t.id, procs: below(&rows, pid) }))
        .collect();
    Reply::Procs { terms }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_processes_of_a_terminal_are_listed_and_agents_marked() {
        let ps = "  100     1 /bin/zsh\n  101   100 /opt/bin/claude\n  102   101 /usr/bin/node\n  103   102 /usr/local/bin/codex\n  200     1 /usr/bin/codex\n";
        let rows = parse_ps(ps);
        let kids = below(&rows, 100);
        let names: Vec<(&str, bool)> = kids.iter().map(|p| (p.name.as_str(), p.agent)).collect();
        assert_eq!(names, vec![("claude", true), ("node", false), ("codex", true)]);
        assert!(below(&rows, 200).is_empty());
    }
}
