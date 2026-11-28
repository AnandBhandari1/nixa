use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

pub const PEER_MS: u64 = 5000;
pub const LSOF: &str = "/usr/sbin/lsof";
pub const PS: &str = "/bin/ps";
pub const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ran {
    pub code: i32,
    pub stdout: String,
}

pub async fn exec(bin: &str, args: &[String]) -> Ran {
    let mut command = tokio::process::Command::new(bin);
    command.args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true);
    match tokio::time::timeout(Duration::from_millis(PEER_MS), command.output()).await {
        Ok(Ok(out)) => Ran { code: out.status.code().unwrap_or(-1), stdout: String::from_utf8_lossy(&out.stdout).into_owned() },
        _ => Ran { code: -1, stdout: String::new() },
    }
}

pub fn parse_lsof_pids(stdout: &str) -> Vec<u32> {
    stdout.lines().map(str::trim).filter_map(|l| l.strip_prefix('p')).filter_map(|n| n.parse().ok()).collect()
}

pub fn parse_executable(stdout: &str) -> Option<String> {
    let lines: Vec<&str> = stdout.lines().map(str::trim).collect();
    let at = lines.iter().position(|l| *l == "ftxt")?;
    lines[at + 1..].iter().find_map(|l| l.strip_prefix('n')).map(str::to_string)
}

pub fn parse_name(stdout: &str) -> Option<String> {
    stdout.lines().map(str::trim).find_map(|l| l.strip_prefix('n')).filter(|n| !n.is_empty()).map(str::to_string)
}

pub fn parse_parents(stdout: &str) -> HashMap<u32, u32> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid = parts.next()?.parse().ok()?;
            let parent = parts.next()?.parse().ok()?;
            parts.next().is_none().then_some((pid, parent))
        })
        .collect()
}

pub fn chain_of(parents: &HashMap<u32, u32>, pid: u32) -> Vec<u32> {
    let mut chain = Vec::new();
    let mut current = Some(pid);
    while let Some(now) = current {
        if now == 0 || chain.contains(&now) || chain.len() >= MAX_DEPTH {
            break;
        }
        chain.push(now);
        current = parents.get(&now).copied();
    }
    chain
}

pub fn real_or(path: &str) -> String {
    std::fs::canonicalize(path).map(|p| p.display().to_string()).unwrap_or_else(|_| path.to_string())
}

pub fn program_name(path: &str) -> String {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or_default().to_string()
}

fn valid_port(port: u32) -> bool {
    port > 0 && port <= 65_535
}

pub async fn peer_pid(port: u32, me: u32) -> Option<u32> {
    if !valid_port(port) {
        return None;
    }
    let found = exec(LSOF, &["-nP".into(), format!("-iTCP@127.0.0.1:{port}"), "-sTCP:ESTABLISHED".into(), "-Fp".into()]).await;
    if found.code != 0 {
        return None;
    }
    let mut pids: Vec<u32> = parse_lsof_pids(&found.stdout).into_iter().filter(|p| *p != me).collect();
    pids.sort_unstable();
    pids.dedup();
    (pids.len() == 1).then(|| pids[0])
}

pub async fn executable_of(pid: u32) -> Option<String> {
    let found = exec(LSOF, &["-a".into(), "-p".into(), pid.to_string(), "-d".into(), "txt".into(), "-Fn".into()]).await;
    if found.code != 0 {
        return None;
    }
    parse_executable(&found.stdout).or_else(|| parse_name(&found.stdout))
}

pub async fn parents() -> Option<HashMap<u32, u32>> {
    let table = exec(PS, &["-A".into(), "-o".into(), "pid=,ppid=".into()]).await;
    (table.code == 0).then(|| parse_parents(&table.stdout))
}

pub async fn is_app(port: u32, app_path: &str, me: u32) -> bool {
    if app_path.trim().is_empty() {
        return false;
    }
    let Some(pid) = peer_pid(port, me).await else { return false };
    let Some(exe) = executable_of(pid).await else { return false };
    real_or(&exe) == real_or(app_path)
}

pub async fn in_terminal(port: u32, root: Option<u32>, me: u32) -> bool {
    let Some(root) = root.filter(|r| *r > 1) else { return false };
    let Some(pid) = peer_pid(port, me).await else { return false };
    parents().await.is_some_and(|table| chain_of(&table, pid).contains(&root))
}

pub fn unseen_text(terminal: &str) -> String {
    format!("Nixa cannot see which program sent this call, so it does not count as terminal {terminal}.")
}

pub fn outside_text(terminal: &str) -> String {
    format!("This call does not come from a program inside terminal {terminal}. Run it in that terminal.")
}

pub async fn from_terminal(peer: Option<SocketAddr>, terminal: &str, root: Option<u32>) -> Result<(), String> {
    let Some(addr) = peer.filter(|a| a.ip().is_loopback()) else { return Err(unseen_text(terminal)) };
    if in_terminal(u32::from(addr.port()), root, std::process::id()).await {
        Ok(())
    } else {
        Err(outside_text(terminal))
    }
}

pub async fn program_of(pid: u32) -> Option<String> {
    executable_of(pid).await.map(|exe| program_name(&exe)).filter(|n| !n.is_empty())
}

pub async fn programs_in(root: u32) -> Vec<String> {
    let Some(table) = parents().await else { return Vec::new() };
    let mut below: Vec<u32> = table.iter().filter(|(pid, _)| chain_of(&table, **pid).contains(&root) && **pid != root).map(|(pid, _)| *pid).collect();
    below.sort_unstable();
    let mut names = Vec::new();
    for pid in below.into_iter().take(MAX_DEPTH) {
        if let Some(name) = program_of(pid).await
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    names
}
